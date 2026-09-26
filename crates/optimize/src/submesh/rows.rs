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
