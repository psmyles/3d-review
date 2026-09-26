//! The surface being rebuilt, and everything measured over it.
//!
//! Every stage of the rebuild reads the same five things — where the vertices
//! are, how they are joined, the index over that, how big a face should be here
//! and how much surface each vertex stands for — and passing them as five
//! parameters made every signature in the operation grow past the point of being
//! readable.
//!
//! So they travel together. It is `Copy` and holds only borrows, so handing it
//! down a call chain costs nothing and it can go into a worker thread as it is.
//!
//! What is deliberately *not* here is [`super::features::Features`]. The feature
//! chains are computed *from* a surface, so a surface cannot contain them
//! without being half-built for the one stage that makes them; and only some of
//! the later stages read them. They stay a separate argument, which is honest
//! about which stages care.

use super::geom;
use super::topology::Topology;

/// The mesh a rebuild is working on.
#[derive(Clone, Copy)]
pub(crate) struct Surface<'a> {
    /// Three floats per vertex.
    pub(crate) positions: &'a [f32],
    /// Three per triangle.
    pub(crate) indices: &'a [u32],
    pub(crate) topology: &'a Topology,
    /// Per vertex: the face size asked for here, in world units.
    pub(crate) sizes: &'a [f32],
    /// Per vertex: the dual area it stands for. What every average over the
    /// surface is weighted by, so a finely triangulated patch does not count for
    /// more than the surface it actually covers.
    pub(crate) areas: &'a [f32],
}

impl<'a> Surface<'a> {
    pub(crate) fn vertex_count(&self) -> usize {
        self.topology.vertex_count
    }

    /// One vertex's position, in `f64` — which is what every accumulation over
    /// the surface works in.
    pub(crate) fn position(&self, vertex: u32) -> [f64; 3] {
        let base = vertex as usize * 3;
        [
            self.positions[base] as f64,
            self.positions[base + 1] as f64,
            self.positions[base + 2] as f64,
        ]
    }

    /// The three corners of a face.
    pub(crate) fn face(&self, face: u32) -> &'a [u32] {
        &self.indices[face as usize * 3..face as usize * 3 + 3]
    }

    /// The distance between two vertices.
    pub(crate) fn distance(&self, a: u32, b: u32) -> f64 {
        let (a, b) = (self.position(a), self.position(b));
        let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        (x * x + y * y + z * z).sqrt()
    }

    /// The dual area of a vertex, never negative.
    pub(crate) fn area(&self, vertex: u32) -> f64 {
        let area = self.areas.get(vertex as usize).copied().unwrap_or(0.0) as f64;
        if area > 0.0 { area } else { 0.0 }
    }

    /// Twice the area of a face, as a vector along its normal.
    pub(crate) fn face_cross(&self, face: u32) -> [f64; 3] {
        let corners = self.face(face);
        geom::triangle_cross(
            self.position(corners[0]),
            self.position(corners[1]),
            self.position(corners[2]),
        )
    }

    /// A face's unit normal, or `None` when it has no area to have one.
    pub(crate) fn face_normal(&self, face: u32) -> Option<[f64; 3]> {
        geom::normalized(self.face_cross(face))
    }

    /// A face's area.
    pub(crate) fn face_area(&self, face: u32) -> f64 {
        geom::length(self.face_cross(face)) * 0.5
    }
}
