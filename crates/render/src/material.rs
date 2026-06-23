//! Editable per-material state + the GPU material table (bind group 3).
//!
//! Phase 1 seeded the renderer with an editable table of per-material PBR
//! parameters drawn one index range per material ([`MaterialDrawRange`]). Phase 3
//! extends each material with **seven texture slots** (base color / normal /
//! roughness / metallic / AO / emissive / opacity) plus **packed-channel routing**:
//! one decoded image (deduplicated by path) can feed several scalar properties via
//! per-property channel selectors carried in the uniform. The GPU side keeps a
//! path-keyed texture cache so a packed map is uploaded once and shared across the
//! materials/slots that reference it; reassigning a slot rebuilds only the bind
//! groups, never the geometry. The UI edits parameters via [`MaterialEdit`] intents
//! and assigns textures via app-side decode (invariant 2).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::texture::{ChannelSelect, DecodedImage, TEXTURE_SLOT_COUNT, TextureSlot};

/// One per-material draw: a contiguous run of the reordered mesh index buffer
/// whose triangles share a single material slot. `material` indexes the table, or
/// `u32::MAX` for triangles that carried no material (drawn with the fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterialDrawRange {
    pub material: u32,
    pub first_index: u32,
    pub index_count: u32,
}

/// How a material's alpha is composited. Driven by an assigned opacity map (or the
/// base color alpha): `Blend` straight-alpha blends, `Clip` does a hard cutout at
/// [`MaterialState::alpha_cutoff`]. `Opaque` ignores alpha entirely.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Blend,
    Clip,
}

impl AlphaMode {
    pub const ALL: [AlphaMode; 3] = [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Clip];

    /// Value the shader branches on (`params.w`).
    fn shader_value(self) -> f32 {
        match self {
            AlphaMode::Opaque => 0.0,
            AlphaMode::Blend => 1.0,
            AlphaMode::Clip => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AlphaMode::Opaque => "Opaque",
            AlphaMode::Blend => "Blend",
            AlphaMode::Clip => "Clip",
        }
    }
}

/// One assigned texture slot: the source path (cache + watcher key), the decoded
/// RGBA8 pixels shared by `Arc` (so the per-frame callback clones a refcount, not
/// megabytes), and which channel(s) feed the property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureBinding {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    pub channel: ChannelSelect,
}

/// Editable per-material PBR parameters — app-side state seeded from the import
/// defaults. `base_color` and `emissive` are linear RGB. The UI edits the scalars
/// via [`MaterialEdit`] intents and assigns the [`TextureBinding`]s via app-side
/// decode; the renderer uploads them into the GPU material table when the material
/// revision changes.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialState {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// The seven texture slots, indexed by [`TextureSlot::index`]. `None` slots use
    /// the shader's neutral per-slot fallback.
    pub textures: [Option<TextureBinding>; TEXTURE_SLOT_COUNT],
    pub alpha_mode: AlphaMode,
    /// Alpha-clip threshold in `0.0..=1.0` (used only in [`AlphaMode::Clip`]).
    pub alpha_cutoff: f32,
}

impl Default for MaterialState {
    fn default() -> Self {
        Self {
            base_color: Vec3::ONE,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
            textures: Default::default(),
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
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

/// Which scalar/color/routing parameter a material edit changes. Colors are linear
/// RGB. Texture *assignment* (which needs a decoded image) flows separately
/// through the app-side decode path, not this enum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaterialChange {
    BaseColor([f32; 3]),
    Metallic(f32),
    Roughness(f32),
    Emissive([f32; 3]),
    /// Re-route an already-assigned slot's channel (the Inspector dropdown). The
    /// `usize` is the [`TextureSlot::index`].
    Channel(usize, ChannelSelect),
    AlphaMode(AlphaMode),
    AlphaCutoff(f32),
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
    /// `x` metallic, `y` roughness, `z` slot_flags bitfield (bit `i` = slot `i`
    /// bound), `w` alpha mode (0 opaque, 1 blend, 2 clip).
    pub params: [f32; 4],
    /// Channel-select index (0 R … 3 A) for slots 0,1,2,3.
    pub channels0: [f32; 4],
    /// Channel-select index for slots 4,5,6 in `x,y,z`; `w` = alpha cutoff.
    pub channels1: [f32; 4],
}

impl MaterialUniform {
    fn from_state(state: &MaterialState) -> Self {
        let mut slot_flags = 0u32;
        let mut channel = [0.0f32; TEXTURE_SLOT_COUNT];
        for (index, binding) in state.textures.iter().enumerate() {
            if let Some(binding) = binding {
                slot_flags |= 1 << index;
                channel[index] = binding.channel.shader_index();
            }
        }
        Self {
            base_color: [
                state.base_color.x,
                state.base_color.y,
                state.base_color.z,
                1.0,
            ],
            emissive: [state.emissive.x, state.emissive.y, state.emissive.z, 0.0],
            params: [
                state.metallic,
                state.roughness,
                slot_flags as f32,
                state.alpha_mode.shader_value(),
            ],
            channels0: [channel[0], channel[1], channel[2], channel[3]],
            channels1: [
                channel[4],
                channel[5],
                channel[6],
                state.alpha_cutoff.clamp(0.0, 1.0),
            ],
        }
    }

    /// Neutral dielectric grey, bound for unmaterialed triangles, overlays, the
    /// grid, skybox and UV draws (none of which sample it). No texture slots bound.
    fn fallback() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8, 1.0],
            emissive: [0.0; 4],
            params: [0.0, 0.5, 0.0, 0.0],
            channels0: [0.0; 4],
            channels1: [0.0; 4],
        }
    }
}

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
    /// Shared sampler for every material texture (repeat, trilinear).
    sampler: wgpu::Sampler,
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
            ..Default::default()
        });
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
                        let view = upload_texture(device, queue, &binding.image, slot.is_srgb());
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

/// Build one material bind group from a uniform slice + seven texture views + the
/// shared sampler.
fn build_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    offset: u64,
    unit: u64,
    sampler: &wgpu::Sampler,
    views: [&wgpu::TextureView; TEXTURE_SLOT_COUNT],
) -> wgpu::BindGroup {
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer,
            offset,
            size: wgpu::BufferSize::new(unit),
        }),
    }];
    for (slot, view) in views.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: 1 + slot as u32,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: 1 + TEXTURE_SLOT_COUNT as u32,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("review_material_bind_group"),
        layout,
        entries: &entries,
    })
}

/// Create the per-slot neutral 1×1 fallback textures (white base/AO/opacity,
/// `[128,128,255]` normal, mid-grey roughness/metallic, black emissive). These are
/// only ever bound for *unassigned* slots, which the shader gates off via the slot
/// flags — but every binding must be filled, so they exist regardless.
fn create_fallback_views(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> [wgpu::TextureView; TEXTURE_SLOT_COUNT] {
    std::array::from_fn(|slot| {
        let texture_slot = TextureSlot::ALL[slot];
        let pixel = fallback_pixel(texture_slot);
        upload_pixels(device, queue, 1, 1, &pixel, texture_slot.is_srgb())
    })
}

/// The neutral fallback RGBA for an unassigned slot.
fn fallback_pixel(slot: TextureSlot) -> [u8; 4] {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Ao | TextureSlot::Opacity => [255, 255, 255, 255],
        TextureSlot::Normal => [128, 128, 255, 255],
        TextureSlot::Roughness | TextureSlot::Metallic => [128, 128, 128, 255],
        TextureSlot::Emissive => [0, 0, 0, 255],
    }
}

/// Upload a decoded image into a GPU texture and return its view. Color slots use
/// `Rgba8UnormSrgb`; linear data slots use `Rgba8Unorm`.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &DecodedImage,
    srgb: bool,
) -> wgpu::TextureView {
    let width = image.width.max(1);
    let height = image.height.max(1);
    // A genuinely empty / mismatched buffer falls back to a single white texel so a
    // decode hiccup never panics inside the render path.
    if image.rgba.len() < (width as usize * height as usize * 4) {
        return upload_pixels(device, queue, 1, 1, &[255, 255, 255, 255], srgb);
    }
    upload_pixels(device, queue, width, height, &image.rgba, srgb)
}

/// Create + fill a 2D RGBA8 texture and return its view.
fn upload_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rgba: &[u8],
    srgb: bool,
) -> wgpu::TextureView {
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_material_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Allocate the strided uniform buffer for `count` entries (materials + fallback).
fn create_uniform_buffer(device: &wgpu::Device, stride: u64, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("review_material_buffer"),
        size: stride * count.max(1) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Smallest multiple of `alignment` that is `>= value`.
fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.div_ceil(alignment) * alignment
}
