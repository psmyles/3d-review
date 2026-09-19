//! The proxy's connectivity, as flat arrays.
//!
//! Every stage of the rebuild asks the same two questions — *what touches this
//! vertex* and *what kind of edge is this* — and each of them used to answer it
//! by building a `HashMap` of its own ([`super::manifold`] built two;
//! `remesh_density.cpp` built a third; [`super::project`] a fourth). This is
//! that structure, built once.
//!
//! ## Why there is no edge table
//!
//! The obvious shape is a `HashMap<(u32, u32), _>` of every undirected edge, and
//! at the sizes this operation is now expected to take it is the one thing that
//! cannot be afforded: a 10M-triangle object has ~15M edges, and a hash entry
//! per edge is a quarter of a gigabyte before a single field is computed.
//!
//! So the only stored adjacency is **vertex to incident face** (one `u32` per
//! face corner, 120 MB at that size and unavoidable — it *is* the connectivity),
//! and an edge is looked up by walking one endpoint's incident faces.
//! [`Topology::edges_at`] does that walk once per vertex and hands back every
//! edge there with its use count and the faces using it, which is what all the
//! per-edge questions are actually asking.
//!
//! ## Why the flags are computed in the same pass
//!
//! A vertex's edge runs give the boundary test, the non-manifold-edge test and
//! the fan test together: sort the vertex's edge partners, and a partner
//! appearing once is a border edge, twice is an interior edge joining two faces
//! into one fan, and three or more is a branch. So one sort per vertex answers
//! what previously took a global edge map plus a per-vertex union-find over it.
//!
//! Each undirected edge is *counted* at its lower-numbered endpoint only, so the
//! totals are exact rather than doubled.

use crate::cancel::{CancelToken, cancelled};
use crate::parallel;

/// One undirected edge as seen from a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EdgeAt {
    /// The far endpoint.
    pub(crate) other: u32,
    /// How many faces use this edge: 1 is a border, 2 an interior edge, 3 or
    /// more a branch no surface can describe.
    pub(crate) uses: u32,
    /// The first two faces using it, in ascending order. A border edge leaves
    /// the second `u32::MAX`. Two is all any caller needs — the dihedral angle
    /// across an edge is between exactly two faces, and an edge with more than
    /// two is a feature by virtue of being one.
    pub(crate) faces: [u32; 2],
}

impl EdgeAt {
    pub(crate) fn is_boundary(self) -> bool {
        self.uses == 1
    }

    pub(crate) fn is_nonmanifold(self) -> bool {
        self.uses > 2
    }
}

/// What one vertex's own edges say about it. Gathered per vertex so the whole
/// analysis is one parallel sweep.
#[derive(Debug, Clone, Copy, Default)]
struct VertexFacts {
    boundary: bool,
    nonmanifold: bool,
    /// Edges whose far endpoint is *higher* numbered, so summing these over
    /// every vertex counts each undirected edge exactly once.
    owned_edges: u32,
    owned_boundary: u32,
    owned_nonmanifold: u32,
}

/// The proxy's connectivity.
///
/// Holds no geometry: positions and the index buffer stay with the proxy and are
/// passed back in where a method needs them (invariant 1). That is also what
/// lets this be cached across runs while the mesh it describes is rebuilt.
#[derive(Debug)]
pub(crate) struct Topology {
    pub(crate) vertex_count: usize,
    pub(crate) face_count: usize,
    /// CSR start offsets into [`Self::face_entries`], `vertex_count + 1` long.
    face_starts: Vec<u32>,
    /// Incident face index per (vertex, incidence), ascending within a vertex.
    face_entries: Vec<u32>,
    /// Per vertex: lies on an open border.
    pub(crate) boundary: Vec<bool>,
    /// Per vertex: on a branching edge, or its faces form more than one fan.
    /// Treated as a feature everywhere downstream — never collapsed into, always
    /// a survivor — so the rebuilt surface keeps a vertex where the input
    /// branched instead of inventing a third face on an edge.
    pub(crate) nonmanifold: Vec<bool>,
    /// Connected component per vertex; `u32::MAX` for a vertex no face uses.
    pub(crate) component: Vec<u32>,
    pub(crate) components: u32,
    /// Vertices at least one face uses. The Euler characteristic counts these,
    /// not the array length.
    pub(crate) referenced: usize,
    pub(crate) edge_count: usize,
    pub(crate) boundary_edges: usize,
    pub(crate) nonmanifold_edges: usize,
    pub(crate) nonmanifold_vertices: usize,
}

impl Topology {
    /// Index the mesh. `indices` is a flat corner stream, three per face.
    pub(crate) fn build(
        indices: &[u32],
        vertex_count: usize,
        threads: usize,
        cancel: Option<&CancelToken>,
    ) -> Topology {
        let _z = crate::prof::zone!("Remesh Topology");
        let faces = indices.as_chunks::<3>().0;

        // Count, prefix-sum, fill — the pattern `ao.rs` and the C bridges use,
        // so the entries array is allocated once rather than grown per vertex.
        let mut face_starts = vec![0u32; vertex_count + 1];
        for corners in faces {
            for &corner in corners {
                if (corner as usize) < vertex_count {
                    face_starts[corner as usize + 1] += 1;
                }
            }
        }
        for vertex in 0..vertex_count {
            face_starts[vertex + 1] += face_starts[vertex];
        }
        let mut face_entries = vec![0u32; face_starts[vertex_count] as usize];
        {
            let mut cursor = face_starts[..vertex_count].to_vec();
            for (face, corners) in faces.iter().enumerate() {
                for &corner in corners {
                    if (corner as usize) < vertex_count {
                        let slot = &mut cursor[corner as usize];
                        face_entries[*slot as usize] = face as u32;
                        *slot += 1;
                    }
                }
            }
        }

        let mut topology = Topology {
            vertex_count,
            face_count: faces.len(),
            face_starts,
            face_entries,
            boundary: vec![false; vertex_count],
            nonmanifold: vec![false; vertex_count],
            component: vec![u32::MAX; vertex_count],
            components: 0,
            referenced: 0,
            edge_count: 0,
            boundary_edges: 0,
            nonmanifold_edges: 0,
            nonmanifold_vertices: 0,
        };
        if cancelled(cancel) {
            return topology;
        }

        topology.analyse(indices, threads);
        topology.label_components(indices);
        topology
    }

    /// Incident faces of `vertex`, ascending.
    pub(crate) fn faces_of(&self, vertex: u32) -> &[u32] {
        let start = self.face_starts[vertex as usize] as usize;
        let end = self.face_starts[vertex as usize + 1] as usize;
        &self.face_entries[start..end]
    }

    /// Whether any face uses `vertex`.
    pub(crate) fn is_referenced(&self, vertex: u32) -> bool {
        !self.faces_of(vertex).is_empty()
    }

    /// Every undirected edge at `vertex`, once each, ascending by far endpoint.
    ///
    /// `scratch` is reused across calls: a sweep over five million vertices must
    /// not allocate five million times.
    pub(crate) fn edges_at(&self, indices: &[u32], vertex: u32, scratch: &mut Vec<EdgeAt>) {
        scratch.clear();
        // Every face corner adjacent to `vertex`, as (far endpoint, face). A
        // vertex of valence n contributes 2n of these, and an interior edge
        // appears twice — once from each of its faces.
        let mut partners: Vec<(u32, u32)> = Vec::new();
        for &face in self.faces_of(vertex) {
            let corners = &indices[face as usize * 3..face as usize * 3 + 3];
            let Some(at) = corners.iter().position(|&corner| corner == vertex) else {
                continue;
            };
            partners.push((corners[(at + 1) % 3], face));
            partners.push((corners[(at + 2) % 3], face));
        }
        partners.sort_unstable();

        let mut index = 0;
        while index < partners.len() {
            let other = partners[index].0;
            let mut uses = 0u32;
            let mut faces = [u32::MAX; 2];
            while index < partners.len() && partners[index].0 == other {
                if (uses as usize) < faces.len() {
                    faces[uses as usize] = partners[index].1;
                }
                uses += 1;
                index += 1;
            }
            // A degenerate face naming `vertex` twice would report an edge to
            // itself; the proxy filters those, and one here would only confuse
            // the fan test.
            if other != vertex {
                scratch.push(EdgeAt { other, uses, faces });
            }
        }
    }

    /// `V - E + F` over the referenced vertices. Two for a sphere, zero for a
    /// torus, one per component for a disc.
    ///
    /// Read by the seed budget, which turns a face count into a vertex count
    /// through it; until that stage lands the tests below are what exercise it,
    /// and they are also what proves the index above is right.
    #[allow(dead_code, reason = "the seed budget is the next stage to land")]
    pub(crate) fn euler(&self) -> i64 {
        self.referenced as i64 - self.edge_count as i64 + self.face_count as i64
    }

    /// One sweep over the vertices: boundary, branch and fan flags, and the
    /// edge totals.
    fn analyse(&mut self, indices: &[u32], threads: usize) {
        let mut facts = vec![VertexFacts::default(); self.vertex_count];
        parallel::sweep(&mut facts, threads, |base, chunk| {
            let mut edges: Vec<EdgeAt> = Vec::new();
            // The local fan union-find, reused down the chunk.
            let mut parent: Vec<u32> = Vec::new();
            for (offset, slot) in chunk.iter_mut().enumerate() {
                let vertex = (base + offset) as u32;
                *slot = self.analyse_vertex(indices, vertex, &mut edges, &mut parent);
            }
        });

        // Merged here rather than inside the sweep: these are counts, and
        // summing them in vertex order is what keeps the totals the same
        // however the chunks were scheduled.
        for (vertex, fact) in facts.iter().enumerate() {
            self.boundary[vertex] = fact.boundary;
            self.nonmanifold[vertex] = fact.nonmanifold;
            self.edge_count += fact.owned_edges as usize;
            self.boundary_edges += fact.owned_boundary as usize;
            self.nonmanifold_edges += fact.owned_nonmanifold as usize;
            if fact.nonmanifold {
                self.nonmanifold_vertices += 1;
            }
            if self.is_referenced(vertex as u32) {
                self.referenced += 1;
            }
        }
    }

    /// One vertex's edges, flags and owned-edge counts.
    fn analyse_vertex(
        &self,
        indices: &[u32],
        vertex: u32,
        edges: &mut Vec<EdgeAt>,
        parent: &mut Vec<u32>,
    ) -> VertexFacts {
        let mut facts = VertexFacts::default();
        let incident = self.faces_of(vertex);
        if incident.is_empty() {
            return facts;
        }
        self.edges_at(indices, vertex, edges);

        // The fan test, over this vertex's own incident faces: two faces join
        // when they share an edge *here*. A branching edge is left out of the
        // join — it is already a defect, and treating it as a link would report
        // fans as connected that the surface does not connect.
        parent.clear();
        parent.extend(0..incident.len() as u32);
        for edge in edges.iter() {
            if edge.other > vertex {
                facts.owned_edges += 1;
                if edge.is_boundary() {
                    facts.owned_boundary += 1;
                }
                if edge.is_nonmanifold() {
                    facts.owned_nonmanifold += 1;
                }
            }
            if edge.is_boundary() {
                facts.boundary = true;
            }
            if edge.is_nonmanifold() {
                facts.nonmanifold = true;
                continue;
            }
            if edge.uses == 2
                && let (Some(a), Some(b)) = (
                    incident.iter().position(|&face| face == edge.faces[0]),
                    incident.iter().position(|&face| face == edge.faces[1]),
                )
            {
                union(parent, a as u32, b as u32);
            }
        }
        let fans = (0..incident.len() as u32)
            .filter(|&slot| find(parent, slot) == slot)
            .count();
        if fans > 1 {
            facts.nonmanifold = true;
        }
        facts
    }

    /// Connected components over the faces, by union-find rather than a flood
    /// fill, so a component's id is a function of the mesh and not of the walk.
    fn label_components(&mut self, indices: &[u32]) {
        let mut parent: Vec<u32> = (0..self.vertex_count as u32).collect();
        for corners in indices.as_chunks::<3>().0 {
            let [a, b, c] = *corners;
            if (a as usize) < self.vertex_count
                && (b as usize) < self.vertex_count
                && (c as usize) < self.vertex_count
            {
                union(&mut parent, a, b);
                union(&mut parent, a, c);
            }
        }
        // Numbered by first appearance in vertex order, so the labels are
        // stable across runs and across thread counts.
        let mut label_of: Vec<u32> = vec![u32::MAX; self.vertex_count];
        for vertex in 0..self.vertex_count as u32 {
            if !self.is_referenced(vertex) {
                continue;
            }
            let root = find(&mut parent, vertex) as usize;
            if label_of[root] == u32::MAX {
                label_of[root] = self.components;
                self.components += 1;
            }
            self.component[vertex as usize] = label_of[root];
        }
    }
}

/// Union-find with path halving, the same shape `project::build_regions` uses.
fn find(parent: &mut [u32], mut node: u32) -> u32 {
    while parent[node as usize] != node {
        parent[node as usize] = parent[parent[node as usize] as usize];
        node = parent[node as usize];
    }
    node
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        // Lower root wins, so the result does not depend on the order the
        // caller happened to visit the pair in.
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        parent[high as usize] = low;
    }
}

#[cfg(test)]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::submesh::partition;

    /// Two triangles sharing a diagonal: an open quad.
    const OPEN_QUAD: [u32; 6] = [0, 1, 2, 0, 2, 3];

    fn build(indices: &[u32], vertex_count: usize) -> Topology {
        Topology::build(indices, vertex_count, 1, None)
    }

    #[test]
    fn a_closed_cube_is_a_manifold_sphere() {
        let model = demo_cube_model();
        let (pieces, _) = partition(&model, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = super::super::proxy::build(&borrowed);

        let topology = build(&proxy.indices, proxy.positions.len() / 3);

        assert_eq!(topology.boundary_edges, 0, "a cube is closed");
        assert_eq!(topology.nonmanifold_edges, 0);
        assert_eq!(topology.nonmanifold_vertices, 0);
        assert_eq!(topology.components, 1);
        assert_eq!(topology.euler(), 2, "a closed cube is a sphere");
        assert!(topology.boundary.iter().all(|&on| !on));
    }

    #[test]
    fn an_open_quad_reports_its_four_border_edges_and_vertices() {
        let topology = build(&OPEN_QUAD, 4);

        assert_eq!(topology.boundary_edges, 4);
        assert_eq!(topology.nonmanifold_edges, 0);
        assert_eq!(topology.edge_count, 5, "four border edges and the diagonal");
        assert_eq!(topology.referenced, 4);
        assert_eq!(topology.euler(), 1, "a disc");
        assert!(
            topology.boundary.iter().all(|&on| on),
            "every corner of an open quad is on its border"
        );
    }

    #[test]
    fn a_third_face_on_one_edge_is_non_manifold() {
        let topology = build(&[0, 1, 2, 0, 1, 3, 0, 1, 4], 5);

        assert_eq!(
            topology.nonmanifold_edges, 1,
            "edge 0-1 is used three times"
        );
        assert!(topology.nonmanifold[0] && topology.nonmanifold[1]);
        assert!(
            !topology.nonmanifold[2],
            "a vertex off the branch is unaffected"
        );
    }

    /// Two triangles meeting at one vertex and nowhere else: a legal edge count
    /// everywhere, but the fan test is what catches it.
    #[test]
    fn a_pinched_vertex_is_non_manifold_though_every_edge_is_a_border() {
        let topology = build(&[0, 1, 2, 0, 3, 4], 5);

        assert_eq!(topology.nonmanifold_edges, 0);
        assert_eq!(topology.nonmanifold_vertices, 1);
        assert!(topology.nonmanifold[0], "vertex 0 carries two fans");
    }

    #[test]
    fn two_separate_triangles_are_two_components() {
        let topology = build(&[0, 1, 2, 3, 4, 5], 6);

        assert_eq!(topology.components, 2);
        assert_eq!(topology.component[0], 0);
        assert_eq!(topology.component[3], 1);
        assert_eq!(topology.euler(), 2, "two discs");
    }

    #[test]
    fn an_unreferenced_vertex_is_not_counted() {
        // Vertex 4 exists in the array and no face uses it.
        let topology = build(&OPEN_QUAD, 5);

        assert_eq!(topology.referenced, 4);
        assert_eq!(topology.component[4], u32::MAX);
        assert!(!topology.is_referenced(4));
    }

    #[test]
    fn edges_at_reports_each_edge_once_with_its_faces() {
        let topology = build(&OPEN_QUAD, 4);
        let mut edges = Vec::new();

        topology.edges_at(&OPEN_QUAD, 0, &mut edges);

        assert_eq!(edges.len(), 3, "vertex 0 touches 1, 2 and 3");
        assert_eq!(
            edges[0],
            EdgeAt {
                other: 1,
                uses: 1,
                faces: [0, u32::MAX]
            }
        );
        assert_eq!(
            edges[1],
            EdgeAt {
                other: 2,
                uses: 2,
                faces: [0, 1]
            },
            "the diagonal is shared by both faces, ascending"
        );
        assert_eq!(
            edges[2],
            EdgeAt {
                other: 3,
                uses: 1,
                faces: [1, u32::MAX]
            }
        );
    }

    #[test]
    fn the_index_is_the_same_at_every_thread_count() {
        // Big enough to span several sweep chunks.
        let rows = 400u32;
        let columns = 400u32;
        let mut indices = Vec::new();
        for row in 0..rows - 1 {
            for column in 0..columns - 1 {
                let at = row * columns + column;
                indices.extend_from_slice(&[at, at + 1, at + columns]);
                indices.extend_from_slice(&[at + 1, at + columns + 1, at + columns]);
            }
        }
        let vertices = (rows * columns) as usize;

        let one = Topology::build(&indices, vertices, 1, None);
        let many = Topology::build(&indices, vertices, 8, None);

        assert_eq!(one.boundary, many.boundary);
        assert_eq!(one.nonmanifold, many.nonmanifold);
        assert_eq!(one.component, many.component);
        assert_eq!(one.edge_count, many.edge_count);
        assert_eq!(one.boundary_edges, many.boundary_edges);
        assert_eq!(
            one.boundary_edges,
            (4 * (rows - 1)) as usize,
            "a grid's border is its four sides"
        );
        assert_eq!(one.euler(), 1, "a grid is a disc");
    }
}
