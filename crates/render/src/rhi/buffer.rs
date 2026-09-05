//! GPU buffers.
//!
//! So far one kind: [`TransientBuffer`], the per-frame geometry stream. Immutable
//! vertex/index buffers and the skinning storage buffers arrive with the scene stages
//! that need them (`mac-port-plan.md` Phase 1 step 4).

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};

/// A buffer written from the CPU once per frame and consumed by the GPU in that same
/// frame — the egui chrome's vertices and indices, which are re-tessellated every
/// frame anyway and never survive one.
///
/// sokol enforces the contract: the write must happen **before** the buffer is bound
/// in the frame, and a frame that binds it without writing it is a validation error
/// rather than a stale draw. That is why the renderer writes in `prepare` (outside
/// any pass) and binds in `paint`.
///
/// It grows by reallocation. egui's vertex count settles within a few frames of a
/// layout change and then never moves, so a growth path measured in "once per resize"
/// beats sizing for a worst case that never happens.
pub(crate) struct TransientBuffer {
    buffer: sg::Buffer,
    /// Capacity in bytes.
    capacity: usize,
    index: bool,
    label: &'static CStr,
}

impl TransientBuffer {
    /// A vertex-stream buffer of `capacity` bytes.
    pub(crate) fn vertices(capacity: usize, label: &'static CStr) -> GpuResult<Self> {
        Self::new(capacity, false, label)
    }

    /// An index-stream buffer of `capacity` bytes.
    pub(crate) fn indices(capacity: usize, label: &'static CStr) -> GpuResult<Self> {
        Self::new(capacity, true, label)
    }

    fn new(capacity: usize, index: bool, label: &'static CStr) -> GpuResult<Self> {
        let capacity = capacity.max(1);
        let mut desc = sg::BufferDesc::new();
        desc.size = capacity;
        desc.usage.write_transient = true;
        desc.usage.index_buffer = index;
        desc.usage.vertex_buffer = !index;
        desc.label = label.as_ptr();
        let buffer = sg::make_buffer(&desc);
        require_valid(
            sg::query_buffer_state(buffer),
            ResourceKind::Buffer,
            label.to_str().unwrap_or("buffer"),
        )?;
        Ok(Self {
            buffer,
            capacity,
            index,
            label,
        })
    }

    /// The sokol handle, for an `sg::Bindings`.
    pub(crate) fn handle(&self) -> sg::Buffer {
        self.buffer
    }

    /// Write this frame's whole payload, reallocating first if it no longer fits.
    ///
    /// Growth rounds up to the next power of two so a slowly growing chrome does not
    /// reallocate every frame.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> GpuResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > self.capacity {
            let grown = bytes.len().next_power_of_two();
            let replacement = Self::new(grown, self.index, self.label)?;
            // Only once the replacement exists, so a failed growth leaves the old
            // buffer usable and the frame merely clipped rather than the viewer dead.
            let old = std::mem::replace(self, replacement);
            drop(old);
        }
        // `dst.offset` must be a multiple of 4; 0 always is.
        let mut desc = sg::WriteBufferDesc::new();
        desc.src = sg::WriteBufferSource {
            data: sg::slice_as_range(bytes),
            offset: 0,
        };
        desc.dst = sg::BufferLocation {
            buffer: self.buffer,
            offset: 0,
        };
        desc.size = bytes.len();
        sg::write_buffer_transient(&desc);
        Ok(())
    }
}

impl Drop for TransientBuffer {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_buffer(self.buffer);
        }
    }
}
