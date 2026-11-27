//! One island as the seam planner sees it: its vertices, its edges with the
//! triangles either side, and which of them a seam may run along.
//!
//! The vertices are the island's cut vertices ([`super::islands`]), so a seam
//! the settings already named is a border here. A new seam is a set of edges
//! marked cut; [`Surface::domain`] splits the vertices along them — two corners
//! share a vertex only where a chain of uncut edges round it joins them, so a
//! seam that stops inside the island is a slit, not a tear — and hands back the
//! chart to flatten.
//!
//! A seam may run only between two faces: never along an open edge (it is a
//! border already) and never inside a polygon (see [`super::islands`]: the
//! triangles of a quad stay together). Those edges are the **cuttable** ones.

use std::collections::HashMap;

use glam::{DVec3, Vec3};

use super::charts::CutMesh;
use super::flatten::Domain;

/// No triangle: the far side of a border edge.
pub(super) const NONE: u32 = u32::MAX;

/// An edge of a surface.
pub(super) struct SurfaceEdge {
    /// Its vertices, lower first.
    pub ends: [u32; 2],
    /// The triangles either side; the second is [`NONE`] on a border (and on
    /// an edge more than two triangles share, which joins none of them).
    pub sides: [u32; 2],
    /// Both sides are triangles of one polygon.
    pub diagonal: bool,
    /// The reference UVs are cut along it, and a seam is preferred there.
    pub preferred: bool,
    pub length: f64,
}

/// An island, ready for seams.
pub(super) struct Surface {
    /// The island's triangles, by their index in the cut mesh, ascending.
    pub triangles: Vec<u32>,
    /// Per vertex, its position.
    pub points: Vec<Vec3>,
    /// Per triangle (in the order of `triangles`), its corners' vertices.
    pub corners: Vec<[u32; 3]>,
    /// Per triangle, its face.
    pub face: Vec<u32>,
    pub edges: Vec<SurfaceEdge>,
    /// Per vertex, its edges.
    vertex_edges: Vec<Vec<u32>>,
    /// Per corner, the first corner of its face at its vertex: corners of one
    /// polygon at one point are one vertex, cut or not.
    face_corner: Vec<u32>,
}

impl Surface {
    /// The surface of `triangles` (indices into `mesh`, ascending).
    pub(super) fn new(mesh: &CutMesh<'_>, triangles: &[u32]) -> Self {
        let mut number: HashMap<u32, u32> = HashMap::new();
        let mut points: Vec<Vec3> = Vec::new();
        let corners: Vec<[u32; 3]> = triangles
            .iter()
            .map(|&triangle| {
                mesh.triangles[triangle as usize].map(|vertex| {
                    *number.entry(vertex).or_insert_with(|| {
                        points.push(mesh.positions[vertex as usize]);
                        points.len() as u32 - 1
                    })
                })
            })
            .collect();
        let face: Vec<u32> = triangles
            .iter()
            .map(|&triangle| mesh.face[triangle as usize])
            .collect();

        // Edges in order of first use; the map is only looked up.
        let mut ids: HashMap<(u32, u32), u32> = HashMap::new();
        let mut users: Vec<Vec<u32>> = Vec::new();
        let mut preferred: Vec<bool> = Vec::new();
        let mut ends: Vec<[u32; 2]> = Vec::new();
        for (local, corners) in corners.iter().enumerate() {
            let bits = mesh.preferred[triangles[local] as usize];
            for k in 0..3 {
                let (a, b) = (corners[k], corners[(k + 1) % 3]);
                if a == b {
                    continue;
                }
                let id = *ids.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    users.push(Vec::new());
                    preferred.push(false);
                    ends.push([a.min(b), a.max(b)]);
                    users.len() as u32 - 1
                });
                users[id as usize].push(local as u32);
                if bits & (1 << k) != 0 {
                    preferred[id as usize] = true;
                }
            }
        }
        let mut vertex_edges: Vec<Vec<u32>> = vec![Vec::new(); points.len()];
        let edges: Vec<SurfaceEdge> = users
            .iter()
            .zip(&ends)
            .zip(&preferred)
            .enumerate()
            .map(|(id, ((users, &[a, b]), &preferred))| {
                vertex_edges[a as usize].push(id as u32);
                vertex_edges[b as usize].push(id as u32);
                let sides = match users.as_slice() {
                    [first, second] => [*first, *second],
                    _ => [users[0], NONE],
                };
                SurfaceEdge {
                    ends: [a, b],
                    sides,
                    diagonal: sides[1] != NONE
                        && face[sides[0] as usize] == face[sides[1] as usize],
                    preferred,
                    length: points[a as usize]
                        .as_dvec3()
                        .distance(points[b as usize].as_dvec3()),
                }
            })
            .collect();

        let mut first: HashMap<(u32, u32), u32> = HashMap::new();
        let face_corner = corners
            .iter()
            .enumerate()
            .flat_map(|(local, corners)| {
                corners
                    .iter()
                    .enumerate()
                    .map(move |(k, &vertex)| (local, k, vertex))
            })
            .map(|(local, k, vertex)| {
                *first
                    .entry((face[local], vertex))
                    .or_insert((local * 3 + k) as u32)
            })
            .collect();
        Self {
            triangles: triangles.to_vec(),
            points,
            corners,
            face,
            edges,
            vertex_edges,
            face_corner,
        }
    }

    /// The edges at `vertex`.
    pub(super) fn edges_at(&self, vertex: u32) -> &[u32] {
        &self.vertex_edges[vertex as usize]
    }

    /// The end of `edge` that is not `vertex`.
    pub(super) fn across(&self, edge: u32, vertex: u32) -> u32 {
        let [a, b] = self.edges[edge as usize].ends;
        if a == vertex { b } else { a }
    }

    /// Whether a seam may run along `edge`: between two faces.
    pub(super) fn cuttable(&self, edge: u32) -> bool {
        let edge = &self.edges[edge as usize];
        edge.sides[1] != NONE && !edge.diagonal
    }

    /// Whether `edge` is a border already.
    pub(super) fn border(&self, edge: u32) -> bool {
        self.edges[edge as usize].sides[1] == NONE
    }

    /// How many faces the surface has.
    pub(super) fn face_count(&self) -> usize {
        let mut faces = self.face.clone();
        faces.sort_unstable();
        faces.dedup();
        faces.len()
    }

    /// Per triangle, its 3D area.
    pub(super) fn areas(&self) -> Vec<f64> {
        self.corners
            .iter()
            .map(|corners| {
                let [a, b, c] = corners.map(|vertex| self.points[vertex as usize].as_dvec3());
                0.5 * (b - a).cross(c - a).length()
            })
            .collect()
    }

    /// Per vertex, how cone-like the surface is round it: the angle deficit
    /// (2π less the angles its triangles meet at) summed over the vertex and
    /// its neighbours, which a scan's noise cannot flip from one vertex to the
    /// next. Positive at a tip, a dome or a corner, where a seam opens a gap;
    /// negative at a saddle, where the two sides of a seam lie over each
    /// other. Meaningful only where the vertex's own ring is closed.
    pub(super) fn cone(&self) -> Vec<f64> {
        let mut deficit = vec![std::f64::consts::TAU; self.points.len()];
        for corners in &self.corners {
            let [a, b, c] = corners.map(|vertex| self.points[vertex as usize].as_dvec3());
            for (at, (p, q, r)) in corners.iter().zip([(a, b, c), (b, c, a), (c, a, b)]) {
                let (u, v) = ((q - p).normalize_or_zero(), (r - p).normalize_or_zero());
                deficit[*at as usize] -= review_model::exact::acos_f64(u.dot(v).clamp(-1.0, 1.0));
            }
        }
        (0..self.points.len() as u32)
            .map(|vertex| {
                deficit[vertex as usize]
                    + self
                        .edges_at(vertex)
                        .iter()
                        .map(|&edge| deficit[self.across(edge, vertex) as usize])
                        .sum::<f64>()
            })
            .collect()
    }

    /// Per vertex, its normal: the area-weighted sum of its triangles'.
    pub(super) fn normals(&self) -> Vec<Vec3> {
        let mut sum = vec![DVec3::ZERO; self.points.len()];
        for corners in &self.corners {
            let [a, b, c] = corners.map(|vertex| self.points[vertex as usize].as_dvec3());
            let normal = (b - a).cross(c - a);
            for &vertex in corners {
                sum[vertex as usize] += normal;
            }
        }
        sum.iter()
            .map(|normal| normal.normalize_or_zero().as_vec3())
            .collect()
    }

    /// The chart the surface is once every edge `cut` marks is opened.
    pub(super) fn domain(&self, cut: &[bool]) -> Domain {
        let mut parent: Vec<u32> = (0..self.corners.len() as u32 * 3).collect();
        for (id, edge) in self.edges.iter().enumerate() {
            let [first, second] = edge.sides;
            if second == NONE || cut[id] {
                continue;
            }
            for vertex in edge.ends {
                union(
                    &mut parent,
                    self.corner(first, vertex),
                    self.corner(second, vertex),
                );
            }
        }
        for (corner, &first) in self.face_corner.iter().enumerate() {
            union(&mut parent, corner as u32, first);
        }
        // Vertices in order of first corner.
        let mut number = vec![u32::MAX; parent.len()];
        let mut points = Vec::new();
        let mut origin = Vec::new();
        let mut triangles = Vec::with_capacity(self.corners.len());
        for (local, corners) in self.corners.iter().enumerate() {
            let mut triangle = [0u32; 3];
            for (k, &vertex) in corners.iter().enumerate() {
                let at = root(&mut parent, (local * 3 + k) as u32) as usize;
                if number[at] == u32::MAX {
                    number[at] = points.len() as u32;
                    points.push(self.points[vertex as usize]);
                    origin.push(vertex);
                }
                triangle[k] = number[at];
            }
            triangles.push(triangle);
        }
        Domain {
            points,
            triangles,
            origin,
        }
    }

    /// The surface's connected pieces through its uncut edges, each as cut
    /// mesh triangles (ascending), in order of first triangle.
    pub(super) fn components(&self) -> Vec<Vec<u32>> {
        let mut parent: Vec<u32> = (0..self.corners.len() as u32).collect();
        for edge in &self.edges {
            if edge.sides[1] != NONE {
                union(&mut parent, edge.sides[0], edge.sides[1]);
            }
        }
        // A polygon is one piece even where it touches itself only at a corner.
        let mut first: HashMap<u32, u32> = HashMap::new();
        for (local, &face) in self.face.iter().enumerate() {
            let first = *first.entry(face).or_insert(local as u32);
            union(&mut parent, first, local as u32);
        }
        let mut number: HashMap<u32, usize> = HashMap::new();
        let mut pieces: Vec<Vec<u32>> = Vec::new();
        for local in 0..self.corners.len() as u32 {
            let at = root(&mut parent, local);
            let piece = *number.entry(at).or_insert_with(|| {
                pieces.push(Vec::new());
                pieces.len() - 1
            });
            pieces[piece].push(self.triangles[local as usize]);
        }
        pieces
    }

    /// The corner of `triangle` at `vertex`.
    fn corner(&self, triangle: u32, vertex: u32) -> u32 {
        let k = self.corners[triangle as usize]
            .iter()
            .position(|&at| at == vertex)
            .unwrap_or(0);
        triangle * 3 + k as u32
    }
}

pub(super) fn root(parent: &mut [u32], mut at: u32) -> u32 {
    while parent[at as usize] != at {
        parent[at as usize] = parent[parent[at as usize] as usize];
        at = parent[at as usize];
    }
    at
}

/// Join two sets; the lower root wins, so the result does not depend on the
/// order the joins are made in.
pub(super) fn union(parent: &mut [u32], a: u32, b: u32) {
    let (x, y) = (root(parent, a), root(parent, b));
    if x != y {
        parent[x.max(y) as usize] = x.min(y);
    }
}
