//! What a draw reads: the vertex and index buffers, the texture views and the
//! samplers, as one value.
//!
//! sokol takes bindings as a *value* re-applied after every `apply_pipeline`, rather
//! than as sticky slot state the way D3D11's context did. That is why this is a plain
//! struct the caller fills and hands over, and why the old `bind_*` / `unbind_*` pairs
//! have no successor: there is nothing left bound at the end of a pass to unbind.
//!
//! It also wraps `sg::Bindings` rather than exposing it, so a texture is bound by
//! handing over the [`Texture`] itself and no `sg::View` ever escapes `rhi`.

use sokol::gfx as sg;

use super::buffer::{IndexBuffer, StorageBuffer, TransientBuffer, VertexBuffer};
use super::sampler::Sampler;
use super::target::ColorTarget;
use super::texture::Texture;

/// The resources one draw reads. Slot numbers are the shdc-generated `VIEW_*` /
/// `SMP_*` constants, so the shader source is what decides them.
#[derive(Clone, Copy)]
pub(crate) struct Bindings(sg::Bindings);

impl Bindings {
    /// Empty bindings — nothing bound in any slot.
    pub(crate) fn new() -> Self {
        Self(sg::Bindings::new())
    }

    /// Bind the interleaved vertex stream (buffer slot 0, the only one the renderer
    /// uses) at a byte offset into it.
    pub(crate) fn vertices(&mut self, buffer: &TransientBuffer, byte_offset: usize) {
        self.0.vertex_buffers[0] = buffer.handle();
        self.0.vertex_buffer_offsets[0] = byte_offset as i32;
    }

    /// Bind the index stream at a byte offset into it.
    pub(crate) fn indices(&mut self, buffer: &TransientBuffer, byte_offset: usize) {
        self.0.index_buffer = buffer.handle();
        self.0.index_buffer_offset = byte_offset as i32;
    }

    /// Bind an immutable mesh vertex buffer (buffer slot 0).
    pub(crate) fn mesh_vertices(&mut self, buffer: &VertexBuffer) {
        self.0.vertex_buffers[0] = buffer.handle();
        self.0.vertex_buffer_offsets[0] = 0;
    }

    /// Bind an immutable index buffer.
    pub(crate) fn mesh_indices(&mut self, buffer: &IndexBuffer) {
        self.0.index_buffer = buffer.handle();
        self.0.index_buffer_offset = 0;
    }

    /// Bind a texture to a view slot.
    pub(crate) fn texture(&mut self, slot: usize, texture: &Texture) {
        self.0.views[slot] = texture.view();
    }

    /// Bind an offscreen colour target to a view slot, for a later pass to sample.
    pub(crate) fn target(&mut self, slot: usize, target: &ColorTarget) {
        self.0.views[slot] = target.texture();
    }

    /// Bind a read-only storage buffer to a view slot — the deform tables the vertex
    /// stage indexes into.
    pub(crate) fn storage<T: bytemuck::Pod>(&mut self, slot: usize, buffer: &StorageBuffer<T>) {
        self.0.views[slot] = buffer.view();
    }

    /// Bind a sampler to a sampler slot.
    pub(crate) fn sampler(&mut self, slot: usize, sampler: &Sampler) {
        self.0.samplers[slot] = sampler.handle();
    }

    /// Bind the bake's render-target cube as a sampled source — the one resource
    /// that is both attachment and texture, and the reason the IBL convolutions
    /// exist at all.
    #[cfg(feature = "bake")]
    pub(crate) fn cube_target(&mut self, slot: usize, target: &super::bake::CubeTarget) {
        self.0.views[slot] = target.texture();
    }

    /// The sokol value, for [`super::Frame::apply_bindings`].
    pub(in crate::rhi) fn raw(&self) -> &sg::Bindings {
        &self.0
    }
}
