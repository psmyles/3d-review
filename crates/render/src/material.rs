//! Editable per-material state + the GPU material table (bind group 3).
//!
//! Phase 1 of the materials system: the renderer owns an editable table of
//! per-material PBR parameters seeded from the import defaults ([`MaterialState`]),
//! uploads them into a single dynamic-offset uniform buffer, and draws one index
//! range per material ([`MaterialDrawRange`]). The UI edits the parameters via
//! [`MaterialEdit`] intents (invariant 2). Textures arrive in Phase 3.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// One per-material draw: a contiguous run of the reordered mesh index buffer
/// whose triangles share a single material slot. `material` indexes the table, or
/// `u32::MAX` for triangles that carried no material (drawn with the fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterialDrawRange {
    pub material: u32,
    pub first_index: u32,
    pub index_count: u32,
}

/// Editable per-material PBR parameters — app-side state seeded from the import
/// defaults. `base_color` and `emissive` are linear RGB. The UI edits these via
/// [`MaterialEdit`] intents; the renderer uploads them into the GPU material
/// table each time the material revision changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialState {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
}

impl Default for MaterialState {
    fn default() -> Self {
        Self {
            base_color: Vec3::ONE,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
        }
    }
}

/// A named editable material for the app→UI snapshot (invariant 2: a plain value
/// the UI reads, never a handle into renderer state).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSnapshot {
    pub name: String,
    pub state: MaterialState,
}

/// Which scalar/color parameter a material edit changes. Colors are linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaterialChange {
    BaseColor([f32; 3]),
    Metallic(f32),
    Roughness(f32),
    Emissive([f32; 3]),
}

/// A UI edit intent: change one parameter of the material at `index`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialEdit {
    pub index: usize,
    pub change: MaterialChange,
}

/// GPU-side per-material uniform (bind group 3, binding 0). `#[repr(C)]` + `Pod`
/// to match the WGSL `MaterialUniform` exactly (invariant 11).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct MaterialUniform {
    pub base_color: [f32; 4],
    pub emissive: [f32; 4],
    /// `x` metallic, `y` roughness, `z` slot_flags (Phase 3), `w` highlight
    /// (Phase 2). Carried now so the layout is stable as later phases land.
    pub params: [f32; 4],
}

impl MaterialUniform {
    fn from_state(state: &MaterialState) -> Self {
        Self {
            base_color: [
                state.base_color.x,
                state.base_color.y,
                state.base_color.z,
                1.0,
            ],
            emissive: [state.emissive.x, state.emissive.y, state.emissive.z, 0.0],
            params: [state.metallic, state.roughness, 0.0, 0.0],
        }
    }

    /// Neutral dielectric grey, bound for unmaterialed triangles, overlays, the
    /// grid, skybox and UV draws (none of which sample it).
    fn fallback() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8, 1.0],
            emissive: [0.0; 4],
            params: [0.0, 0.5, 0.0, 0.0],
        }
    }
}

/// The bind-group layout for group 3. Phase 1 is a single dynamic-offset uniform;
/// Phase 3 extends it with the 7 texture slots + a sampler.
pub(crate) fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("review_material_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(
                    std::mem::size_of::<MaterialUniform>() as u64
                ),
            },
            count: None,
        }],
    })
}

/// Editable material table: a single dynamic-offset uniform buffer holding one
/// [`MaterialUniform`] per material plus a trailing fallback, with one bind group
/// addressed per draw by dynamic offset.
pub(crate) struct MaterialTable {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Aligned stride between consecutive materials (>= the uniform size and a
    /// multiple of the device's min uniform-buffer offset alignment).
    stride: u64,
    /// Number of real materials, *excluding* the trailing fallback entry.
    material_count: usize,
}

impl MaterialTable {
    /// An empty table (fallback only), built before any model is loaded.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        alignment: u64,
    ) -> Self {
        Self::build(device, queue, layout, alignment, &[])
    }

    /// Bring the table in line with `materials`: rebuild the buffer + bind group
    /// when the material count changed (a new model), otherwise just re-upload the
    /// edited values into the existing buffer.
    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        alignment: u64,
        materials: &[MaterialState],
    ) {
        if materials.len() != self.material_count {
            *self = Self::build(device, queue, layout, alignment, materials);
        } else {
            self.upload(queue, materials);
        }
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        alignment: u64,
        materials: &[MaterialState],
    ) -> Self {
        let unit = std::mem::size_of::<MaterialUniform>() as u64;
        let stride = align_up(unit, alignment.max(unit));
        let count = materials.len() + 1;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_material_buffer"),
            size: stride * count as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_material_bind_group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(unit),
                }),
            }],
        });
        let table = Self {
            buffer,
            bind_group,
            stride,
            material_count: materials.len(),
        };
        table.upload(queue, materials);
        table
    }

    /// Pack every material (plus the trailing fallback) into the dynamic-offset
    /// buffer in one write.
    fn upload(&self, queue: &wgpu::Queue, materials: &[MaterialState]) {
        let unit = std::mem::size_of::<MaterialUniform>();
        let count = materials.len() + 1;
        let mut bytes = vec![0u8; self.stride as usize * count];
        for (index, material) in materials.iter().enumerate() {
            let uniform = MaterialUniform::from_state(material);
            let offset = index * self.stride as usize;
            bytes[offset..offset + unit].copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        let fallback = MaterialUniform::fallback();
        let offset = (count - 1) * self.stride as usize;
        bytes[offset..offset + unit].copy_from_slice(bytemuck::bytes_of(&fallback));
        queue.write_buffer(&self.buffer, 0, &bytes);
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub fn material_count(&self) -> usize {
        self.material_count
    }

    /// Dynamic offset (bytes) for a material slot, falling back to the trailing
    /// fallback entry for `u32::MAX` or any out-of-range slot.
    pub fn offset_for(&self, slot: u32) -> u32 {
        let index = if (slot as usize) < self.material_count {
            slot as usize
        } else {
            self.material_count
        };
        (index as u64 * self.stride) as u32
    }

    /// Dynamic offset (bytes) of the fallback entry, bound for draws that don't
    /// sample a real material (overlays, grid, skybox, UV, SSAO G-buffer).
    pub fn fallback_offset(&self) -> u32 {
        (self.material_count as u64 * self.stride) as u32
    }
}

/// Smallest multiple of `alignment` that is `>= value`.
fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.div_ceil(alignment) * alignment
}
