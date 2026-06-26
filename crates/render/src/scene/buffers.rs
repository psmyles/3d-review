//! Buffer + texture + bind-group construction for the scene: the mesh/index/line
//! vertex buffers, the GTAO target textures and their bind groups, the one-shot
//! target clear, the fullscreen pass helper, and the baked UV-checker textures.
//! Pure builders, no `SceneResources` state.

use wgpu::util::DeviceExt;

use crate::gtao::{GTAO_FORMAT, GtaoPass};
use crate::targets::{SCENE_HDR_FORMAT, SceneTargets};

use super::SCENE_DEPTH_FORMAT;
use super::SceneVertex;

pub(super) fn create_mesh_buffers(
    device: &wgpu::Device,
    vertices: &[SceneVertex],
    indices: &[u32],
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let placeholder_vertex = [SceneVertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: [0.0, 0.0, 0.0, 0.0],
    }];
    let placeholder_index = [0_u32];

    let vertex_contents = if vertices.is_empty() {
        bytemuck::cast_slice(&placeholder_vertex)
    } else {
        bytemuck::cast_slice(vertices)
    };
    let index_contents = if indices.is_empty() {
        bytemuck::cast_slice(&placeholder_index)
    } else {
        bytemuck::cast_slice(indices)
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_mesh_vertex_buffer"),
        contents: vertex_contents,
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_mesh_index_buffer"),
        contents: index_contents,
        usage: wgpu::BufferUsages::INDEX,
    });

    (vertex_buffer, index_buffer, indices.len() as u32)
}

/// Create a standalone index buffer (returning its index count). Used for the
/// solo (isolate) draw list, which shares the steady-state mesh vertex buffer but
/// supplies its own reordered, selection-only indices. A placeholder index keeps
/// the buffer non-empty when nothing is selected.
pub(super) fn create_index_buffer(device: &wgpu::Device, indices: &[u32]) -> (wgpu::Buffer, u32) {
    let placeholder_index = [0_u32];
    let contents = if indices.is_empty() {
        bytemuck::cast_slice(&placeholder_index)
    } else {
        bytemuck::cast_slice(indices)
    };
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_selection_index_buffer"),
        contents,
        usage: wgpu::BufferUsages::INDEX,
    });
    (index_buffer, indices.len() as u32)
}

pub(super) fn create_line_buffer(
    device: &wgpu::Device,
    vertices: &[SceneVertex],
) -> (wgpu::Buffer, u32) {
    let placeholder_vertex = [SceneVertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: [0.0, 0.0, 0.0, 0.0],
    }];
    let contents = if vertices.is_empty() {
        bytemuck::cast_slice(&placeholder_vertex)
    } else {
        bytemuck::cast_slice(vertices)
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_debug_line_vertex_buffer"),
        contents,
        usage: wgpu::BufferUsages::VERTEX,
    });

    (vertex_buffer, vertices.len() as u32)
}

/// Create one full-resolution single-channel AO texture (render target + sampled
/// in later passes) and return its view.
fn create_gtao_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GTAO_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// Create the single-sample GTAO G-buffer color+depth targets.
fn create_gtao_gbuffer_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::TextureView, wgpu::TextureView) {
    let extent = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let gbuffer = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_gtao_gbuffer"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_gtao_depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (
        gbuffer.create_view(&wgpu::TextureViewDescriptor::default()),
        depth.create_view(&wgpu::TextureViewDescriptor::default()),
    )
}

/// (Re)create the full-resolution GTAO G-buffer + AO textures and clear the
/// sampled ones. Pass-independent, so the core first-frame build can make them
/// before the deferred GTAO pass exists (Phase B); [`build_gtao_targets`] adds
/// the bind groups once the pass lands. Returns `(gbuffer, depth, raw, blur)`.
pub(super) fn build_gtao_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &SceneTargets,
) -> (
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::TextureView,
) {
    let (gbuffer, depth) = create_gtao_gbuffer_targets(device, targets.width, targets.height);
    let raw = create_gtao_texture(device, targets.width, targets.height, "review_gtao_raw");
    let blur = create_gtao_texture(device, targets.width, targets.height, "review_gtao_blur");
    // The composite binds the blurred AO every frame but only samples it when
    // GTAO is on, so the sampled views are cleared once here to satisfy D3D12's
    // read-before-init validation.
    clear_views(device, queue, &[&gbuffer, &raw, &blur], "review_gtao_init");
    (gbuffer, depth, raw, blur)
}

/// (Re)create the full-resolution GTAO G-buffer, AO textures and bind groups.
/// The occlusion pass reads the single-sample G-buffer → `raw`; the bilateral
/// blur reads both the same G-buffer and `raw` → `blur`. Called whenever scene
/// targets are recreated while the GTAO pass is built.
pub(super) fn build_gtao_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &SceneTargets,
    gtao: &GtaoPass,
) -> (
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::BindGroup,
) {
    let (gbuffer, depth, raw, blur) = build_gtao_textures(device, queue, targets);
    let gtao_bg = gtao.occlusion_bind_group(device, &gbuffer, &blur, "review_gtao_bg");
    let blur_bg = gtao.blur_bind_group(device, &gbuffer, &raw, "review_gtao_blur_bg");
    (gbuffer, depth, raw, blur, gtao_bg, blur_bg)
}

/// Clear a set of color views once (`LoadOp::Clear` to black), in a single
/// throwaway encoder. Used to initialise render targets that a later pass binds
/// but may not write before first read (the GTAO textures when AO is off).
fn clear_views(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    views: &[&wgpu::TextureView],
    label: &str,
) {
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    for view in views {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    queue.submit(std::iter::once(encoder.finish()));
}

/// Run one fullscreen pass: clear `target`, bind `pipeline` + `bind_group`, draw
/// the fullscreen triangle. The GTAO textures are single-sample with no depth, so
/// the pass is a bare color attachment. `timestamp_writes` is `Some` only while
/// the Tracy GPU profiler is active (otherwise `None`, exactly as before).
pub(super) fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    label: &str,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

pub(super) fn create_checker_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    png_bytes: &[u8],
    label: &str,
) -> wgpu::BindGroup {
    // The PNGs are baked in at build time (invariant: assets via include_bytes!),
    // so a decode failure is a packaging bug — fall back to a 1x1 white texel
    // rather than panicking inside the render callback.
    let rgba = image::load_from_memory(png_bytes)
        .map(|image| image.to_rgba8())
        .unwrap_or_else(|_| image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])));
    let (width, height) = rgba.dimensions();

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
        &rgba,
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

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
