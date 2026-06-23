//! Generic GPU texture + bind-group plumbing for the material table: uploading a
//! decoded image into a mip-mapped texture, the per-slot 1×1 fallbacks, the
//! per-material bind-group assembly, and the strided uniform-buffer allocation.
//! Kept separate from the table's policy so a future texture path (the Tex
//! viewport, more formats) can reuse it.

use crate::mipmap::{MipGenerator, mip_level_count};
use crate::texture::{DecodedImage, TEXTURE_SLOT_COUNT, TextureSlot};

/// Build one material bind group from a uniform slice + seven texture views + the
/// shared sampler.
pub(super) fn build_bind_group(
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
pub(super) fn create_fallback_views(
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

/// Upload a decoded image into a mip-mapped GPU texture and return its view. Color
/// slots use `Rgba8UnormSrgb`; linear data slots use `Rgba8Unorm`. A full mip chain
/// is built by [`MipGenerator`] so anisotropic minification has levels to filter.
pub(super) fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mip_generator: &mut MipGenerator,
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

    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let mip_levels = mip_level_count(width, height);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_material_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // RENDER_ATTACHMENT lets the generator blit each downsampled level into place.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    write_mip0(queue, &texture, width, height, &image.rgba);
    mip_generator.generate(device, queue, &texture, format, mip_levels);
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Create + fill a single-level 2D RGBA8 texture (no mips) and return its view.
/// Used for the 1×1 per-slot fallbacks and the empty-buffer guard.
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
    write_mip0(queue, &texture, width, height, rgba);
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Write `rgba` into mip level 0 of `texture`.
fn write_mip0(queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32, rgba: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
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
}

/// Allocate the strided uniform buffer for `count` entries (materials + fallback).
pub(super) fn create_uniform_buffer(
    device: &wgpu::Device,
    stride: u64,
    count: usize,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("review_material_buffer"),
        size: stride * count.max(1) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Smallest multiple of `alignment` that is `>= value`.
pub(super) fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.div_ceil(alignment) * alignment
}
