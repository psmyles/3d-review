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

use super::buffer::TransientBuffer;
use super::sampler::Sampler;
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

    /// Bind a texture to a view slot.
    pub(crate) fn texture(&mut self, slot: usize, texture: &Texture) {
        self.0.views[slot] = texture.view();
    }

    /// Bind a sampler to a sampler slot.
    pub(crate) fn sampler(&mut self, slot: usize, sampler: &Sampler) {
        self.0.samplers[slot] = sampler.handle();
    }

    /// The sokol value, for [`super::Frame::apply_bindings`].
    pub(in crate::rhi) fn raw(&self) -> &sg::Bindings {
        &self.0
    }
}
