//! GPU buffers: the per-frame geometry stream ([`TransientBuffer`]), the immutable
//! mesh buffers ([`VertexBuffer`] / [`IndexBuffer`]) and the read-only storage
//! buffers the vertex shader deforms through ([`StorageBuffer`]).
//!
//! All four are the same sokol object with different usage flags, so they share
//! [`fn@make`] and differ only in what they promise the caller.

use std::ffi::CStr;
use std::marker::PhantomData;

use bytemuck::Pod;
use sokol::gfx as sg;

use super::error::{GpuError, GpuResult, ResourceKind};
use super::make;

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
        let buffer = make::buffer(&desc, label.to_str().unwrap_or("buffer"))?;
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

/// An immutable vertex buffer, uploaded once and drawn from until it is dropped —
/// the mesh, the grid, and every derived line/fill view.
///
/// It carries its own vertex count so a draw cannot disagree with it: every caller
/// draws the whole buffer, and the count is what `Frame::draw` is given.
pub(crate) struct VertexBuffer {
    buffer: sg::Buffer,
    count: usize,
}

impl VertexBuffer {
    /// Upload `vertices`. Rejects an empty set: sokol has no zero-sized buffer, and
    /// a view with nothing in it is expressed as `None` rather than as an empty
    /// buffer (`scene::resources::optional_vertex_buffer`).
    pub(crate) fn new<T: Pod>(vertices: &[T], label: &CStr) -> GpuResult<Self> {
        let mut usage = sg::BufferUsage::new();
        usage.vertex_buffer = true;
        usage.immutable = true;
        Ok(Self {
            buffer: make(bytemuck::cast_slice(vertices), usage, label)?,
            count: vertices.len(),
        })
    }

    /// The sokol handle, for an `sg::Bindings`.
    pub(crate) fn handle(&self) -> sg::Buffer {
        self.buffer
    }

    /// How many vertices to draw.
    pub(crate) fn count(&self) -> usize {
        self.count
    }
}

impl Drop for VertexBuffer {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_buffer(self.buffer);
        }
    }
}

/// An immutable `u32` index buffer, uploaded with the mesh it indexes (or with a
/// derived draw list over the same vertices — the selection and visibility lists).
pub(crate) struct IndexBuffer {
    buffer: sg::Buffer,
    count: usize,
}

impl IndexBuffer {
    pub(crate) fn new(indices: &[u32], label: &CStr) -> GpuResult<Self> {
        let mut usage = sg::BufferUsage::new();
        usage.index_buffer = true;
        usage.immutable = true;
        Ok(Self {
            buffer: make(bytemuck::cast_slice(indices), usage, label)?,
            count: indices.len(),
        })
    }

    pub(crate) fn handle(&self) -> sg::Buffer {
        self.buffer
    }

    /// How many indices the whole buffer holds.
    pub(crate) fn count(&self) -> usize {
        self.count
    }
}

impl Drop for IndexBuffer {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_buffer(self.buffer);
        }
    }
}

/// A read-only storage buffer the vertex stage indexes into — the four deform tables
/// at view slots 12..15.
///
/// Two flavours, because two of the tables are properties of the *model* (the
/// influence runs and the blend-shape deltas, uploaded once with the mesh) and two
/// are properties of the *pose* (the joint palette and the shape weights, rewritten
/// when the pose revision moves). Both are the same sokol object; only the usage
/// differs.
pub(crate) struct StorageBuffer<T> {
    buffer: sg::Buffer,
    view: sg::View,
    /// Elements the buffer was sized for. A dynamic upload shorter than this is
    /// allowed (the tail keeps whatever was there, and the shader never reads it);
    /// a longer one is refused.
    capacity: usize,
    label: &'static CStr,
    element: PhantomData<T>,
}

impl<T: Pod> StorageBuffer<T> {
    /// Upload a table that never changes.
    pub(crate) fn immutable(elements: &[T], label: &'static CStr) -> GpuResult<Self> {
        let mut usage = sg::BufferUsage::new();
        usage.storage_buffer = true;
        usage.immutable = true;
        Self::build(bytemuck::cast_slice(elements), elements.len(), usage, label)
    }

    /// Allocate a table of `capacity` elements to be rewritten with
    /// [`Self::update`]. Its initial contents are zero, and nothing reads it before
    /// the first update — which is what `DeformGpu::palette_revision` tracks.
    pub(crate) fn dynamic(capacity: usize, label: &'static CStr) -> GpuResult<Self> {
        let mut usage = sg::BufferUsage::new();
        usage.storage_buffer = true;
        usage.dynamic_update = true;
        let bytes = capacity
            .checked_mul(size_of::<T>())
            .ok_or_else(|| GpuError::invalid_arg(format!("'{}' overflows", name(label))))?;
        Self::build(&vec![0u8; bytes], capacity, usage, label)
    }

    fn build(
        bytes: &[u8],
        capacity: usize,
        usage: sg::BufferUsage,
        label: &'static CStr,
    ) -> GpuResult<Self> {
        // sokol requires a storage buffer's size to be a multiple of 4; every element
        // type here is a multiple of 4 bytes, so the only way to break it is an empty
        // table — which callers express as `None` (or as the one-element dummy the
        // scene binds when a model has no deform tables at all).
        if bytes.is_empty() {
            return Err(GpuError::invalid_arg(format!(
                "'{}' would be an empty storage buffer",
                name(label)
            )));
        }
        let buffer = if usage.immutable {
            make(bytes, usage, label)?
        } else {
            let mut desc = sg::BufferDesc::new();
            desc.size = bytes.len();
            desc.usage = usage;
            desc.label = label.as_ptr();
            make::buffer(&desc, name(label))?
        };

        let mut view_desc = sg::ViewDesc::new();
        view_desc.storage_buffer.buffer = buffer;
        view_desc.label = label.as_ptr();
        // The buffer above is this constructor's own: a failed view has already
        // freed its handle, but the buffer it was to be a view of is still live.
        let view = match make::view(&view_desc, ResourceKind::Buffer, name(label)) {
            Ok(view) => view,
            Err(error) => {
                sg::destroy_buffer(buffer);
                return Err(error);
            }
        };
        Ok(Self {
            buffer,
            view,
            capacity,
            label,
            element: PhantomData,
        })
    }

    /// Rewrite a dynamic table's contents. Once per frame at most — sokol's
    /// `sg_update_buffer` is a whole-buffer, once-per-frame operation, which is
    /// exactly what a pose upload is.
    pub(crate) fn update(&self, elements: &[T]) -> GpuResult<()> {
        if elements.len() > self.capacity {
            return Err(GpuError::invalid_arg(format!(
                "'{}' holds {} elements but was given {}",
                name(self.label),
                self.capacity,
                elements.len()
            )));
        }
        if elements.is_empty() {
            return Ok(());
        }
        sg::update_buffer(self.buffer, &sg::slice_as_range(elements));
        Ok(())
    }

    /// Elements the buffer was sized for — how a pose is checked against the model
    /// it was built for.
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// The view a binding slot reads it through.
    pub(in crate::rhi) fn view(&self) -> sg::View {
        self.view
    }
}

impl<T> Drop for StorageBuffer<T> {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_view(self.view);
            sg::destroy_buffer(self.buffer);
        }
    }
}

/// Create an immutable buffer from `bytes`, or fail naming it.
fn make(bytes: &[u8], usage: sg::BufferUsage, label: &CStr) -> GpuResult<sg::Buffer> {
    if bytes.is_empty() {
        return Err(GpuError::invalid_arg(format!(
            "'{}' would be an empty buffer",
            name(label)
        )));
    }
    let mut desc = sg::BufferDesc::new();
    desc.size = bytes.len();
    desc.usage = usage;
    desc.data = sg::slice_as_range(bytes);
    desc.label = label.as_ptr();
    make::buffer(&desc, name(label))
}

fn name(label: &CStr) -> &str {
    label.to_str().unwrap_or("buffer")
}
