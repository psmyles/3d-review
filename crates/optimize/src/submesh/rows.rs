//! The per-vertex rows a vertex remap has to follow: skin weights, extra skin
//! layers, DQ weights, blend-shape offsets.
//!
//! A remap **gathers** these from a representative old vertex; a scatter would
//! let the last writer win. That is exact only because the weld key carries a
//! row id, so no two vertices with different rows are ever merged.

use glam::Vec3;

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

    pub(crate) fn push_row(&mut self, entries: &[T]) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.data.extend_from_slice(entries);
        self.offsets.push(self.data.len() as u32);
    }

    /// The rows of `representative[slot]` for every new slot, in order.
    pub(super) fn gather(&self, representative: &[u32]) -> Self {
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

/// Every custom attribute's words for each vertex, side by side in the order
/// of the source's `ModelData::custom_attributes` — vertex `v` holds
/// `words[v * width..(v + 1) * width]`. Width 0 (the default) when the source
/// carried none.
///
/// Unlike the colour channels these are never blended or refit: a remap
/// gathers them, a lent vertex copies them, and the weld key carries them
/// ([`Submesh::row_ids`](super::Submesh::row_ids)), so two vertices that differ
/// in any word stay two vertices — which is what makes the simplifier keep the
/// line between them as a seam.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomWords {
    width: usize,
    words: Vec<u32>,
}

impl CustomWords {
    /// An empty table of `width` words per vertex.
    pub fn with_width(width: usize) -> Self {
        Self {
            width,
            words: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// How many vertices the table covers.
    pub fn len(&self) -> usize {
        self.words.len().checked_div(self.width).unwrap_or(0)
    }

    /// Vertex `vertex`'s words; zeros past the end, so a lookup never fails.
    pub fn row(&self, vertex: usize) -> &[u32] {
        self.words
            .get(vertex * self.width..(vertex + 1) * self.width)
            .unwrap_or(&ZEROS[..self.width.min(ZEROS.len())])
    }

    /// Append one vertex's words; a short row is padded with zeros.
    pub(crate) fn push_row(&mut self, row: &[u32]) {
        if self.width == 0 {
            return;
        }
        let start = self.words.len();
        self.words.extend(row.iter().take(self.width));
        self.words.resize(start + self.width, 0);
    }

    /// The words of `representative[slot]` for every new slot, in order;
    /// [`u32::MAX`] (a slot nothing maps to) gets zeros.
    pub(super) fn gather(&self, representative: &[u32]) -> Self {
        let mut out = Self::with_width(self.width);
        if self.width == 0 {
            return out;
        }
        out.words.reserve(representative.len() * self.width);
        for &old in representative {
            match old {
                u32::MAX => out.words.resize(out.words.len() + self.width, 0),
                old => out.push_row(self.row(old as usize)),
            }
        }
        out
    }
}

/// What [`CustomWords::row`] answers for a vertex past the end: glTF caps an
/// attribute at four components, and nothing reads more than a few of them.
const ZEROS: [u32; 64] = [0; 64];
