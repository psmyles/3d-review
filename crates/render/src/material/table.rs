//! The GPU material table (bind group 3): one [`MaterialUniform`] per material in
//! a strided uniform buffer, one bind group per material (its uniform slice +
//! texture views + sampler), and a path-keyed GPU texture cache so a packed map is
//! uploaded once and shared. Also builds the group-3 bind-group layout.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::mipmap::MipGenerator;
use crate::texture::{TEXTURE_SLOT_COUNT, TextureSlot};

use super::state::{MaterialState, MaterialUniform};
use super::upload::{
    align_up, build_bind_group, create_fallback_views, create_uniform_buffer, upload_texture,
};

/// Anisotropic-filter sample count for the material sampler. 16× is the common
/// hardware ceiling; `wgpu` clamps it to the adapter's actual maximum. Combined
/// with the per-texture mip chain ([`MipGenerator`]) this removes the shimmer that
/// single-level minification produced on grazing-angle surfaces.
const MATERIAL_ANISOTROPY: u16 = 16;

/// The bind-group layout for group 3: the per-material uniform plus the seven
/// texture slots and one shared sampler.
pub(crate) fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<MaterialUniform>() as u64),
        },
        count: None,
    }];
    // Bindings 1..=7: one float texture per slot.
    for slot in 0..TEXTURE_SLOT_COUNT {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1 + slot as u32,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
    }
    // Binding 8: the shared material sampler.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 1 + TEXTURE_SLOT_COUNT as u32,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });

    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("review_material_layout"),
        entries: &entries,
    })
}

/// One cached GPU texture: its view plus the source `Arc`'s pointer (the identity
/// used to detect a disk reload, which swaps in a fresh `Arc` at the same path).
struct CachedTexture {
    identity: usize,
    view: wgpu::TextureView,
}

/// Editable material table: one [`MaterialUniform`] per material (plus a trailing
/// fallback) in a strided uniform buffer, one bind group per material binding its
/// uniform slice + its (possibly shared) texture views + the sampler, and a
/// path-keyed GPU texture cache so a packed map is uploaded once.
pub(crate) struct MaterialTable {
    buffer: wgpu::Buffer,
    /// Aligned stride between consecutive materials (>= the uniform size and a
    /// multiple of the device's min uniform-buffer offset alignment).
    stride: u64,
    /// Number of real materials, *excluding* the trailing fallback entry.
    material_count: usize,
    /// Shared sampler for every material texture (repeat, trilinear + anisotropic).
    sampler: wgpu::Sampler,
    /// Builds the mip chain for each uploaded slot texture (mip-mapped sampling +
    /// anisotropy is what actually resolves minification noise).
    mip_generator: MipGenerator,
    /// Per-slot neutral 1×1 textures, bound for unassigned slots.
    fallback_views: [wgpu::TextureView; TEXTURE_SLOT_COUNT],
    /// GPU texture views keyed by `(path, srgb)`: a decoded image is uploaded once
    /// per color-space it is sampled in and shared by every material/slot at that
    /// path (the app decodes once per path, so they share one `Arc`). Each entry
    /// also stores the source `Arc`'s pointer; a disk reload swaps in a *fresh*
    /// `Arc` at the same path, so the pointer changes and the entry re-uploads
    /// rather than serving the stale view.
    texture_cache: HashMap<(PathBuf, bool), CachedTexture>,
    /// One bind group per material (binds its uniform slice + texture views).
    bind_groups: Vec<wgpu::BindGroup>,
    /// All-fallback bind group, bound for draws that don't sample a real material
    /// (overlays, grid, skybox, UV, SSAO G-buffer).
    fallback_bind_group: wgpu::BindGroup,
}

impl MaterialTable {
    /// An empty table (fallback only), built before any model is loaded.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        alignment: u64,
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_material_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: MATERIAL_ANISOTROPY,
            ..Default::default()
        });
        let mip_generator = MipGenerator::new(device);
        let fallback_views = create_fallback_views(device, queue);

        let unit = std::mem::size_of::<MaterialUniform>() as u64;
        let stride = align_up(unit, alignment.max(unit));
        let buffer = create_uniform_buffer(device, stride, 1);

        // The all-fallback bind group (fallback uniform slice + fallback views),
        // built inline so the struct starts in a valid state.
        let fallback_refs: [&wgpu::TextureView; TEXTURE_SLOT_COUNT] =
            std::array::from_fn(|slot| &fallback_views[slot]);
        let fallback_bind_group =
            build_bind_group(device, layout, &buffer, 0, unit, &sampler, fallback_refs);

        let table = Self {
            buffer,
            stride,
            material_count: 0,
            sampler,
            mip_generator,
            fallback_views,
            texture_cache: HashMap::new(),
            bind_groups: Vec::new(),
            fallback_bind_group,
        };
        // Seed the fallback uniform entry (index 0 of the single-entry buffer).
        table.upload_uniforms(queue, &[]);
        table
    }

    /// Bring the table in line with `materials`: rebuild the uniform buffer (and
    /// drop the GPU texture cache) when the material count changes (a new model),
    /// re-upload the uniforms either way, and rebuild the per-material bind groups
    /// (uploading any newly-referenced texture once, deduplicated by path).
    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        alignment: u64,
        materials: &[MaterialState],
    ) {
        if materials.len() != self.material_count {
            let unit = std::mem::size_of::<MaterialUniform>() as u64;
            self.stride = align_up(unit, alignment.max(unit));
            self.buffer = create_uniform_buffer(device, self.stride, materials.len() + 1);
            self.material_count = materials.len();
            // The previous model's textures no longer apply; reclaim the GPU cache
            // (the new model's slots upload on demand below).
            self.texture_cache.clear();
        }
        self.upload_uniforms(queue, materials);
        self.rebuild_bind_groups(device, queue, layout, materials);
    }

    /// Pack every material (plus the trailing fallback) into the strided uniform
    /// buffer in one write.
    fn upload_uniforms(&self, queue: &wgpu::Queue, materials: &[MaterialState]) {
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

    /// Ensure every referenced texture is uploaded (deduplicated by `(path, srgb)`)
    /// then build one bind group per material plus the all-fallback bind group.
    fn rebuild_bind_groups(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        materials: &[MaterialState],
    ) {
        // 1. Upload any newly-referenced image once; an unchanged cached upload is
        //    skipped, a reloaded one (fresh `Arc` at the same path) is replaced.
        for material in materials {
            for slot in TextureSlot::ALL {
                if let Some(binding) = &material.textures[slot.index()] {
                    let key = (binding.path.clone(), slot.is_srgb());
                    let identity = Arc::as_ptr(&binding.image) as usize;
                    let stale = self
                        .texture_cache
                        .get(&key)
                        .is_none_or(|cached| cached.identity != identity);
                    if stale {
                        let view = upload_texture(
                            device,
                            queue,
                            &mut self.mip_generator,
                            &binding.image,
                            slot.is_srgb(),
                        );
                        self.texture_cache
                            .insert(key, CachedTexture { identity, view });
                    }
                }
            }
        }

        // 2. Build one bind group per material, binding its uniform slice + the
        //    (possibly shared) texture views or the per-slot fallback.
        let unit = std::mem::size_of::<MaterialUniform>() as u64;
        let mut bind_groups = Vec::with_capacity(materials.len());
        for (index, material) in materials.iter().enumerate() {
            let offset = index as u64 * self.stride;
            let views: [&wgpu::TextureView; TEXTURE_SLOT_COUNT] =
                std::array::from_fn(|slot| match &material.textures[slot] {
                    Some(binding) => {
                        let srgb = TextureSlot::ALL[slot].is_srgb();
                        let key = (binding.path.clone(), srgb);
                        self.texture_cache
                            .get(&key)
                            .map(|cached| &cached.view)
                            .unwrap_or(&self.fallback_views[slot])
                    }
                    None => &self.fallback_views[slot],
                });
            bind_groups.push(build_bind_group(
                device,
                layout,
                &self.buffer,
                offset,
                unit,
                &self.sampler,
                views,
            ));
        }
        self.bind_groups = bind_groups;

        // 3. The all-fallback bind group (fallback uniform slice + fallback views).
        let fallback_views: [&wgpu::TextureView; TEXTURE_SLOT_COUNT] =
            std::array::from_fn(|slot| &self.fallback_views[slot]);
        self.fallback_bind_group = build_bind_group(
            device,
            layout,
            &self.buffer,
            self.material_count as u64 * self.stride,
            unit,
            &self.sampler,
            fallback_views,
        );
    }

    pub fn material_count(&self) -> usize {
        self.material_count
    }

    /// Bind group for a material slot, falling back to the all-fallback group for
    /// `u32::MAX` or any out-of-range slot.
    pub fn bind_group_for(&self, slot: u32) -> &wgpu::BindGroup {
        self.bind_groups
            .get(slot as usize)
            .unwrap_or(&self.fallback_bind_group)
    }

    /// The all-fallback bind group, bound for draws that don't sample a real
    /// material (overlays, grid, skybox, UV, SSAO G-buffer).
    pub fn fallback_bind_group(&self) -> &wgpu::BindGroup {
        &self.fallback_bind_group
    }
}
